# Pila

[![CI](https://github.com/JohannesKast/pila/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/JohannesKast/pila/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/JohannesKast/pila/graph/badge.svg)](https://codecov.io/gh/JohannesKast/pila)
[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)

A self-hostable FIFA World Cup 2026 prediction game (Tippspiel) written in Rust.
Friends and family submit exact-score tips for every match plus a champion pick,
collect points, and watch the leaderboard move. Built to run on a homelab box
with Docker Compose.

Pila is built around a simple idea: provide a fair prediction game that people
can join without account registration, password friction, or advertising. At
the same time, it should be a page players actually like returning to during
the tournament, with badges, jerseys, an optional RSS feed, and room for
further fun additions that do not compromise the core game. For the person
hosting it, the goal is equally pragmatic: low setup and maintenance overhead,
so they can do something nice for their group without taking on much operator
burden.

I started this project personally to learn more about agentic coding and to
spend time working with Rust and sqlx on a real, self-contained application
instead of a toy example.

## Features

- **Per-match score tipping** for the entire tournament (group stage through
  final) plus a single Weltmeister/champion pick.
- **Points-based scoring** with a fixed table per tournament phase. Default
  (exact-score) mode: exact result / correct goal difference / correct tendency
  / wrong → 4-3-2-0 (group), 6-4-3-0 (R32/R16), 8-6-5-0 (QF/SF),
  11-8-6-0 (3rd place/final). Each league can optionally switch to a simpler
  winner-only mode (tip home win, draw, or away win) worth 1–7 points depending
  on the round.
- **Multi-tenancy (Tipp-Ligen)**: run several isolated leagues on one instance,
  each with its own users, leaderboard, notification channel, and default language.
- **No accounts, no passwords**: each user gets a personal link. Open it and
  you're in — anyone with the link can tip, so treat it like a password and
  share it privately.
- **No ads, no signup funnel**: the app stays focused on the game instead of
  monetisation clutter.
- **Notifications** (both optional): remind the group when a match or the
  champion pick is about to lock — via a Signal group message and/or by email.
  Quiet hours 22:00–08:00 Europe/Berlin.
- **Internationalised UI** in German, English, Spanish, French.
- **Live score sync** via ESPN's scoreboard, polled every 30 minutes.
- **Hero panel with badges** — purely cosmetic gamification, computed on the fly.
- **RSS news ticker** (optional) for a feed of your choice on the index page.
- **Hardened container by default**: read-only root FS, all capabilities dropped,
  CPU/memory/PID limits, healthchecks.

## Tech Stack

Axum + sqlx (PostgreSQL 15) + Askama templates + HTMX, served from a single
binary. Signal notifications go through `bbernhard/signal-cli-rest-api`.

## Requirements

- Linux host with Docker and Docker Compose v2
- ~512 MB RAM and 1 CPU core free for the app container
- A public hostname + reverse proxy if you want to expose the app to the
  internet (recommended: Caddy, Traefik, or nginx with Let's Encrypt)
- Optional: a dedicated Signal phone number for the bot
- Optional: an email account for outgoing notifications

## Installation (Homelab Quickstart)

```bash
git clone https://github.com/<your-fork>/pila.git
cd pila

# 1. Configure environment
cp .env.example .env
# Edit .env — at minimum set POSTGRES_PASSWORD (strong random) and BASE_URL.
$EDITOR .env

# 2. Start the stack (database + signal-cli + app)
docker compose up -d
docker compose logs -f app
```

The app listens on `http://localhost:8000`. Migrations and first-run bootstrap
run automatically on startup — no manual database setup needed. On first start
the admin setup page will walk you through creating the first league and your
personal invite link.

### Notifications (optional)

Pila can reach out to players in two ways — both are optional and independent:

**Signal**: ping a group when a match or the champion pick locks soon. You need
a dedicated phone number registered with Signal. Set `SIGNAL_FROM_NUMBER` and
`SIGNAL_GROUP_ID` in `.env` (or per-league from the admin UI), then link the
number to the bundled `signal-cli` container:

```bash
# Temporarily expose the signal-cli REST API to localhost for registration
# (uncomment the ports: line in docker-compose.yml for signal-cli, then restart)
docker compose up -d signal-cli
# Register your number, verify via SMS, add the bot to your group.
# Full guide: https://github.com/bbernhard/signal-cli-rest-api
```

**Email**: set `SMTP_HOST`, `SMTP_PORT`, `SMTP_USER`, `SMTP_PASS`, and
`SMTP_FROM` in `.env`. Any standard mail provider works (Gmail app password,
Fastmail, your homelab MTA, etc.). If the vars are absent, Pila skips email
delivery and only shows invite links in the admin UI for manual sharing.

## Recommended Secure Deployment

Pila is small and pragmatic — most of the heavy lifting is done by your
reverse proxy and the kernel.

### Reverse proxy with TLS

Do **not** expose port 8000 directly. Front the app with Caddy, Traefik, or
nginx, terminate TLS there, and let the proxy talk to the app over the Docker
network. Example Caddy snippet:

```caddyfile
pila.example.com {
    encode zstd gzip
    reverse_proxy pila_app:8000
    header {
        Strict-Transport-Security "max-age=31536000; includeSubDomains; preload"
        X-Content-Type-Options "nosniff"
        Referrer-Policy "strict-origin-when-cross-origin"
        Permissions-Policy "interest-cohort=()"
    }
}
```

Set `BASE_URL=https://pila.example.com` in `.env` so invite links and
notifications use the public hostname.

### Secrets

- Generate `POSTGRES_PASSWORD` with `openssl rand -base64 32` and store `.env`
  with `chmod 600`.
- Keep `.env` out of git (already in `.gitignore`).
- Invite links are bearer credentials — share them via Signal, a password
  manager, or another private channel, not in a public chat.

### Container hardening

The default `docker-compose.yml` already drops all Linux capabilities, sets
`no-new-privileges`, mounts the root FS read-only, and limits CPU/memory/PIDs.
Don't relax those without a reason.

### Network exposure

- Bind the reverse proxy to your WAN; keep everything else on the Docker
  internal network.
- Do **not** expose the `signal-cli` port in production — it has no auth. The
  port comment in `docker-compose.yml` is there for the one-time registration
  step only.
- For LAN-only access, bind the proxy to a private interface and skip the
  public DNS record entirely.

### Backups

```bash
./backup_db.sh                  # gzip dump into ./backups/
./restore_db.sh backups/<file>  # restore
```

Schedule `backup_db.sh` via cron or a systemd timer and copy dumps off the
host. The database is the only stateful piece worth backing up.

### Updates

```bash
git pull
docker compose build app
docker compose up -d app
```

Migrations run automatically on startup. Review the diff before pulling — this
is a hobby project, not LTS software.

## Local Development

```bash
docker compose up -d db
cp .env.example .env  # set POSTGRES_PASSWORD
DATABASE_URL=postgres://pila:<pw>@localhost:6433/pila_db cargo run
```

```bash
cargo test       # unit + integration tests
cargo clippy
cargo sqlx prepare   # after any sqlx::query! change — commit .sqlx/
```

See [`doc/architecture.md`](doc/architecture.md) for the full architecture overview.

## License

[GNU Affero General Public License v3.0 or later](LICENSE).

Pila is free software: you can redistribute it and/or modify it under the
terms of the AGPL-3.0-or-later. In short: if you run a modified version on a
network server, you must offer its source code to the users interacting with
it. See the LICENSE file for the full text.
