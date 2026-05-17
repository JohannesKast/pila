# Contributing to Pila

Thank you for your interest in contributing to Pila — a self-hostable FIFA World Cup prediction game.

## Local Development Setup

### Prerequisites

- [rustup](https://rustup.rs/) with the **stable** toolchain
- [sqlx-cli](https://github.com/launchbear/sqlx-cli): `cargo install sqlx-cli --no-default-features --features native-tls,postgres`
- [Docker Compose v2](https://docs.docker.com/compose/install/) (`docker compose` — not the legacy `docker-compose`)

### Getting started

```bash
git clone https://github.com/JohannesKast/pila.git
cd pila

# Copy and fill in the required env vars
cp .env.example .env

# Start Postgres only
docker compose up -d db

# Run the app against the local DB
DATABASE_URL=postgres://pila:<pw>@localhost:6433/pila_db cargo run
```

The app listens on `http://localhost:8000` by default (override via `PORT`).

### Full stack (app + signal-cli)

```bash
docker compose up -d
docker compose logs -f app
```

## Issues and Pull Requests

- Everyone may open issues for bugs, feature requests, docs gaps, and
  operational feedback.
- External code contributions should come from a fork and be proposed as a pull
  request against `master`.
- Please open an issue before spending significant time on a larger feature so
  the scope can be aligned early.
- The maintainer decides what gets merged and when. Opening a PR does not imply
  acceptance.

## sqlx Offline Mode

All `sqlx::query!` / `query_as!` macros are checked at compile time against
`.sqlx/` — a checked-in snapshot of the DB schema. The Docker build sets
`SQLX_OFFLINE=true`, so **whenever you add, change, or remove a sqlx macro you
must regenerate the snapshot and commit it**:

```bash
DATABASE_URL=postgres://pila:<pw>@localhost:6433/pila_db cargo sqlx prepare
git add .sqlx/
```

If you skip this step the Docker build will fail.

## Mandatory Gates (run before every PR)

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo sqlx prepare      # only if any sqlx::query! changed; commit .sqlx/
cargo audit             # no unpatched advisories
cargo deny check        # license + source allowlist
```

CI runs the same checks on every push. A PR can only be merged when all checks
are green.

## Internationalisation (i18n)

Pila supports four locales: `de`, `en`, `es`, `fr` (Mozilla Fluent format,
`locales/*.ftl`).

**Rule:** any new user-visible string must be added to **all four** FTL files in
the same commit with a `# translator comment` that explains the context. Strings
hardcoded in a single language are a regression and should be rejected in review.

## Commit Message Style

Pila uses [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):

```
<type>(<optional scope>): <short summary>

[optional body]

[optional footer(s)]
```

Common types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`.

- Subject line ≤ 72 characters
- Use the imperative mood ("add", not "added")
- Reference issues or PRs in the footer when relevant: `Closes #42`

## Branch Model

- **Trunk-based development** on `master`
- If you are contributing from a fork, create your feature branch there:
  `git checkout -b feat/my-feature`
- Open a pull request from your branch into `master`
- Rebase before merging; no merge commits on `master`

## Multi-Tenancy Invariants

Every aggregate query (leaderboard, badges, notifications, admin user lists)
**must** be scoped by `league_id`. Adding a new query that reads across all
leagues is a regression. See `tests/multi_league_isolation.rs` — extend it for
any new aggregate.

## Code Style

- English only in source code, comments, tests, migrations, and documentation
- No bare `.unwrap()` in response/handler paths — use `?`, `expect("reason")`,
  or proper error handling
- Default to writing no comments; add one only when the *why* is non-obvious
