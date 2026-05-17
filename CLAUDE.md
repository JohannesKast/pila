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

## Change Rules

- Run `cargo clippy --all-targets -- -D warnings` after substantive changes.
- If you change a `sqlx::query!`/`query_as!` invocation, run `cargo sqlx prepare`
  against a live `DATABASE_URL` and commit the `.sqlx/` changes.
- Extend tests when adding multi-tenant aggregate queries; use
  `tests/multi_league_isolation.rs` as the regression net.

## Reference Docs

- Human-readable architecture overview: [`doc/architecture.md`](doc/architecture.md)
- Scoreboard provider contract: [`doc/scoreboard_provider.md`](doc/scoreboard_provider.md)
- Contributor workflow and local setup: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- Product/runtime overview: [`README.md`](README.md)
