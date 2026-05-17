# TODO — Road to v0.1 and Public Release

Working document. Tasks are grouped into four sprints, ordered by dependency.
Tick boxes as you finish. Each task lists *files touched*, *acceptance
criteria*, and (where relevant) *test gate*. Keep PRs small — one task ≈ one
PR is a good default.

Legend:
- **[Blocker]** must be done before the next sprint starts
- **[Public-blocker]** must be done before flipping the repo to public
- **[Polish]** nice-to-have, defer if time-boxed

---

## Sprint 1 — Code Hygiene (Pre-v0.1)

Goal: codebase obeys its own rules (CLAUDE.md), no panics in response paths,
repository modules are split into trait / pg / fake.

### 1.1 — Move hardcoded German strings into FTL locales [DONE]

- [x] Handler error responses → FTL via `t_err` (commit cdc4f1a)
- [x] Notifier + mail rendering → FTL with per-league and per-recipient bundles (commit c9b8db5)
- [x] Badge titles and tooltips → FTL via `badge-<key>-{title,how-to-earn}` (commit 8bb0ab5)
- [x] Jersey preset display names → English in code; user-visible names already came from `jersey-name-{key}` FTL keys
- [x] German doc-comments translated to English (CLAUDE.md compliance)

**Acceptance achieved:** `grep -rE '"[äöüßÄÖÜ]' src/{handlers,notifier.rs,mail.rs,badges.rs,jersey.rs}` returns no hits; clippy clean; all 222 tests green.

**Test gate still open** (deferred to Sprint 1.5 — "Close test coverage gaps"): integration tests that assert the English variant of each migrated handler response when `users.language = 'en'`.

### 1.2 — Eliminate `.unwrap()` in response paths [DONE]

- [x] `src/handlers/jersey.rs:175` — already infallible via `HeaderValue::from_static("/")` (verified, no change needed).
- [x] `src/handlers/services.rs:192` — replaced `unwrap()` with `jersey::get()` helper (existing `expect()` documents the invariant). Added a startup assertion in `jersey::load()` that the "classic" key exists.
- [x] Sweep: `src/main.rs` `security_headers_middleware` no longer `.parse().unwrap()`s static literals on every response — switched to `HeaderValue::from_static`. `src/mail.rs` static content-type → `.expect()`. `src/handlers/dev.rs` `next_match.kickoff_time.unwrap()` documented with `.expect("filter above guarantees kickoff_time is Some")`.

**Acceptance achieved:** No bare `.unwrap()` left in response/handler paths. Remaining `.unwrap()`s in `src/main.rs` are startup-only (DB pool connect, listener bind, axum::serve) — exempt. Worker/notifier production paths are clean (only test-gated unwraps remain). `cargo clippy --all-targets -- -D warnings` clean; 222 tests green.

### 1.3 — Split repo modules: trait / postgres / memory [PARTIAL]

Four files mixed trait definition, Postgres impl, in-memory fake, and tests:

- [x] `src/repo/user.rs` → `src/repo/user/{mod.rs, postgres.rs, memory.rs}`
- [x] `src/repo/match_.rs` → `src/repo/match_/{mod.rs, postgres.rs, memory.rs}`
- [x] `src/repo/notification.rs` → `src/repo/notification/{mod.rs, postgres.rs, memory.rs}`
- [x] `src/repo/prediction.rs` → `src/repo/prediction/{mod.rs, postgres.rs, memory.rs}`

Pattern landed for each module:
```
src/repo/<name>/
  mod.rs        — trait, public types, re-exports
  postgres.rs   — Pg<Name>Repo impl
  memory.rs    — Memory<Name>Repo impl + #[cfg(test)] memory_tests submodule
```

Verification: `cargo clippy --all-targets -- -D warnings` clean; all 222 tests green.

**Still open — feature-gate decision:** memory fakes are currently `pub` always so the `tests/` integration tests (which link the lib compiled without `cfg(test)`) can use them. Production binary still contains `Memory*Repo` symbols. Two ways to fix:
- (a) Add `[features] default = ["memory-repos"]; memory-repos = []` + gate every `Memory*` with `#[cfg(any(test, feature = "memory-repos"))]` + ship Dockerfile with `cargo build --release --no-default-features`.
- (b) Leave as is (memory repos in release binary, ~tiny overhead, no behavioural effect).

`nm target/release/pila | grep -i memory` will show `Memory*Repo` symbols until (a) is applied.

### 1.4 — Fix `AppState.db: Option<PgPool>` smell [Polish] [DONE]

- [x] `repo::bootstrap::BootstrapRepo` trait + `FirstLeagueParams` DTO
- [x] `PgBootstrapRepo` encapsulates the multi-table setup transaction
- [x] `MemoryBootstrapRepo` stub for handler tests
- [x] `Repos.bootstrap` field; `Repos::from_pool` wires `PgBootstrapRepo`
- [x] `handlers/setup.rs` uses `state.repos.bootstrap.create_first_league_and_admin`
- [x] `AppState.db` field removed

**Acceptance achieved:** `grep -n 'state\.db' src/` → no hits; clippy clean; 222+ tests green.

### 1.5 — Close test coverage gaps [Blocker] [DONE]

- [x] `tests/handler_setup.rs` — 9 tests: happy path sets `pila_token` cookie; already-done → 403; empty name/league-name → 400; league-name > 255 chars → 400; unknown language → 400; empty language defaults to `de`.
- [x] `tests/handler_language.rs` — 7 tests: all four locales accepted (200 + `HX-Location: /`); unknown + empty locale rejected (400); persistence verified in `MemoryUserRepo`.
- [x] `tests/handler_auth.rs` — 6 tests: magic-link valid token → cookie + redirect; unknown token → 401; `AuthenticatedUser` extractor: valid / missing / unknown cookie → correct results.
- [x] `tests/handler_leaderboard.rs` — 4 tests: renders HTML; includes league-mates; excludes cross-league users (isolation in both directions).
- [x] `tests/handler_services.rs` — 11 tests covering the internal orchestration the handlers share: `fetch_actual_champion` (none on unfinished final / winner on finished); `fetch_leaderboard` (lists zero-point users, sorts by `total_points` desc, unfinished tips add to `max_potential`, jersey preset attached); `fetch_group_standings` (empty when no finished matches, 3-pts-win / 1-pt-draw / 0-pts-loss with goal accumulation, within-group sort by points→goal_diff, multi-group keyed by letter); `build_badge_context` shape.

Also fixed two `MemoryUserRepo` bugs uncovered by new tests:
- `find_by_token` returned hardcoded `language: "de"` instead of the stored value
- `set_language` was a no-op stub; now properly updates the in-memory record

**Acceptance achieved:** `cargo test --all-targets` passes (13 test suites, all green); clippy clean.

### 1.6 — Refactor `dev_simulate_next_matchday` [Polish] [DONE]

Currently 127 lines in `src/handlers/dev.rs:344–470`. Mixes match selection,
RNG, time mutation, and bulk updates.

- [x] Extract `fn find_next_unstarted_matchday(matches: &[Match]) -> Option<Vec<Match>>` (pure, testable)
- [x] Extract `fn random_result(rng: &mut impl Rng) -> (i32, i32)` (pure, testable)
- [x] Unit-test both extracted helpers
- [x] Handler shrinks to orchestration only

**Acceptance achieved:** Handler ≤ 50 lines; two new unit tests in `dev.rs`; clippy clean; all tests green.

### 1.7 — Add missing doc comments on public trait methods [Polish] [DONE]

- [x] `src/repo/league.rs` — `list`, `find_by_id`, `create`, `set_bootstrapped`, `get_config`
- [x] `src/repo/user/mod.rs` — `find_by_token`, `find_full_by_id`, `create`, `delete`, `set_admin`, `set_can_create_league`, `rename`, `set_jersey`, `set_language`, `set_email`
- [x] `src/repo/notification/mod.rs` — verified all methods already documented
- [x] `src/repo/prediction/mod.rs` — `upsert`
- [x] `src/repo/fixture/mod.rs` — `list_for_index`, `find_lock_info`, `first_kickoff`, `first_knockout_kickoff`, `actual_champion`, `finished_group_rows`
- [x] `src/repo/special_prediction.rs` — `get_user_champion`, `upsert`, `user_champion_view`
- [x] `src/repo/team.rs` — verified all methods already documented
- [x] `src/repo/settings.rs` — `get`, `set`

**Acceptance achieved:** All public trait methods documented; clippy clean.

### 1.8 — Sprint 1 wrap-up [Blocker] [DONE]

- [x] Run `cargo clippy --all-targets -- -D warnings` — clean
- [x] Run `cargo fmt` — clean diff
- [x] Run `cargo sqlx prepare` and commit `.sqlx/` if any query changed (`.sqlx/` updated due to fmt reformatting SQL strings)
- [x] Update CHANGELOG.md (create if missing) with v0.1.0-unreleased entries
- [x] Tag a working snapshot: `git tag v0.1.0-rc1` (not pushed)

---

## Sprint 2 — Pre-Release Operations

Goal: deployment works end-to-end, operators have what they need.

### 2.1 — Smoke-test operational scripts [Blocker]

- [ ] `./backup_db.sh` — produces a valid `.sql.gz` dump in `./backups/`
- [ ] `./restore_db.sh backups/<file>` — restores into a fresh DB, app comes up
- [ ] `./create_invite.sh` — with `PILA_BASE_URL=https://example.com`, the printed URL uses the override
- [ ] `./admin_edit.sh` — works on a populated DB (round-trip a prediction edit)
- [ ] `./delete_user.sh` — confirms cascade delete (predictions, special_predictions, sent_notifications cleaned up)
- [ ] `./signal_send.sh` — manually triggers a Signal message to the configured group (or NoopNotifier path if not configured)

**Acceptance:** Each script runs without errors against a real Docker Compose stack.

### 2.2 — Verify Docker hardening still works [Blocker]

The compose file sets read-only root FS, drops caps, limits CPU/mem.

- [ ] `docker compose up -d` — app starts healthy
- [ ] Verify `docker inspect pila_app | jq '.[0].HostConfig.{ReadonlyRootfs,CapDrop,PidsLimit}'` shows the expected restrictions
- [ ] Confirm `/healthz` returns 200 from outside the container
- [ ] Tail logs for 1 hour; confirm no permission-denied or read-only-fs errors

**Acceptance:** Healthy container running with hardened settings; logs clean.

### 2.3 — Create CHANGELOG.md [Blocker]

- [ ] Follow Keep-a-Changelog 1.1.0 format
- [ ] First entry: `## [0.1.0] — 2026-MM-DD` with sections Added / Changed / Fixed / Security
- [ ] Populate from `git log v0.1.0-rc1..HEAD` (or initial set of features)

### 2.4 — Decide migration consolidation [Decision needed]

Six migrations exist (`20260601000000_init` … `20260601000005_user_email`).

- [ ] **Question:** is there a production DB anywhere right now that would break if we squash migrations?
  - If **no**: squash all six into a single `20260601000000_init.sql` for a clean v0.1 starting point. Forks get one migration to read.
  - If **yes**: leave as is, document the chain in `doc/migrations.md`.

### 2.5 — Sprint 2 wrap-up

- [ ] All operational scripts green
- [ ] Tag `v0.1.0` (still not pushed publicly)
- [ ] Internal deploy to homelab, run for ≥ 48 h without errors before Sprint 3

---

## Sprint 3 — Public-Readiness

Goal: repo can flip to public without embarrassment or legal/security risk.

### 3.1 — License switch (DONE) [Public-blocker]

- [x] `LICENSE` replaced with AGPL-3.0-or-later
- [x] `README.md` license section updated
- [x] `Cargo.toml` `license = "AGPL-3.0-or-later"`
- [ ] Add SPDX header to every Rust source file:
      `// SPDX-License-Identifier: AGPL-3.0-or-later`
      `// Copyright (C) 2026 Johannes Kast`
      (consider a script: `for f in src/**/*.rs; do …; done`)
- [ ] Add a short AGPL notice at the bottom of every full-page rendered template (link to source repo, satisfies "Appropriate Legal Notices" in AGPL §5(d) when running over a network)

**Acceptance:** All Rust files have SPDX header; rendered pages include a "Source code" link.

### 3.2 — Contributor documentation [Public-blocker]

- [ ] `CONTRIBUTING.md`:
  - Local dev setup (Docker + cargo)
  - Required tools: `rustup` stable, `sqlx-cli`, Docker Compose v2
  - `cargo sqlx prepare` workflow + when to commit `.sqlx/`
  - Mandatory gates per PR: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
  - i18n rule: any new UI string must land in all four locale FTL files
  - Commit message style (caveman/Conventional Commits — decide)
  - Branch model: trunk-based on `master`, PR from feature branch
- [ ] `CODE_OF_CONDUCT.md`: Contributor Covenant 2.1 (verbatim), set contact email
- [ ] `SECURITY.md`:
  - Where to report vulnerabilities (private GitHub Security Advisory link, plus fallback email)
  - SLA: best-effort acknowledgement within 7 days
  - Disclosure policy (90-day default)
- [ ] `.github/PULL_REQUEST_TEMPLATE.md`: summary, motivation, test plan, screenshots (if UI)
- [ ] `.github/ISSUE_TEMPLATE/bug_report.yml` — structured form
- [ ] `.github/ISSUE_TEMPLATE/feature_request.yml` — structured form
- [ ] `.github/ISSUE_TEMPLATE/config.yml` — link to Discussions if/when enabled
- [ ] `.github/CODEOWNERS` — `* @JohannesKast`

### 3.3 — Split CLAUDE.md from human docs [Public-blocker]

CLAUDE.md is Claude-Code-specific. Externals need an equivalent.

- [ ] Create `doc/architecture.md` — same content minus AI references:
  - What Pila is
  - Module map (src/ tree)
  - DB schema overview
  - Multi-tenancy invariants
  - Worker / notification flow
  - i18n architecture
  - SQLx offline workflow
- [ ] CLAUDE.md keeps only: AI-collaboration rules, links into `doc/architecture.md`
- [ ] Move `doc/scoreboard_provider.md` reference into `doc/architecture.md` as well

### 3.4 — Clean up `handoff/` directory [DONE]

- [x] `handoff/` directory deleted (was never tracked by git; design artefacts not preserved).

### 3.5 — Scrub git history for secrets [Public-blocker]

- [ ] Run `git log --all -p | grep -iE 'password|secret|token|@gmail\.com|@.*\..*\.com'` — review every hit
- [ ] Run `gitleaks detect --source . --log-opts="--all"` (install via `go install github.com/gitleaks/gitleaks/v8@latest` or use the Docker image)
- [ ] Review `.env.example` for any non-placeholder values
- [ ] Search for hardcoded personal email/phone in fixtures and migrations
- [ ] If any real secret found: rewrite history with `git filter-repo` and rotate the secret. **Do not push** the original branch first.

**Acceptance:** Gitleaks run is clean; manual grep returns no real secrets.

### 3.6 — Harden CI [Public-blocker]

Current CI runs clippy + test. Add:

- [x] `cargo fmt --check` step
- [x] `cargo audit` (RustSec advisory DB) — fail on any unpatched advisory
- [x] `cargo deny check` (license + source allowlist + dupes). Add a `deny.toml` that:
  - Allowlists AGPL-compatible licenses for deps
  - Bans known-bad crates
  - Warns on duplicate versions
- [x] Enable Dependabot (`.github/dependabot.yml`) for `cargo` and `github-actions` ecosystems, weekly schedule
- [x] (Optional) Coverage step with `cargo-llvm-cov` and upload to Codecov

### 3.7 — README polish for public eyeballs [Public-blocker]

- [ ] Add a screenshot or short GIF of the UI at the top
- [ ] Add a "Why Pila?" paragraph (vs. existing prediction games — fewer ads, self-hostable, multi-league, AGPL)
- [ ] Add a "Roadmap" section: what's coming (post-v0.1 ideas) and what's intentionally out of scope
- [x] Add badges row: CI (exists), license (AGPL), Rust edition, GHCR image tag (once built)
- [ ] Verify all internal links work (`CLAUDE.md`, `doc/`, etc.)

### 3.8 — Release artefacts [Public-blocker]

- [ ] GitHub Actions workflow `release.yml`:
  - Trigger on tag push `v*.*.*`
  - Build multi-arch Docker image (`linux/amd64`, `linux/arm64`)
  - Push to `ghcr.io/johanneskast/pila:vX.Y.Z` + `:latest`
  - Generate SBOM with `anchore/sbom-action` (CycloneDX or SPDX)
  - Sign image with cosign (keyless via OIDC)
- [ ] Update README install snippet to pull the image instead of building from source (or offer both)
- [ ] Document the release process in `doc/release.md`: bump Cargo.toml version, update CHANGELOG, tag, push tag → workflow does the rest

### 3.9 — Final pre-public checklist [Public-blocker]

- [ ] Tag pushed: `v0.1.0` exists publicly
- [ ] Release artefact available at GHCR
- [ ] Verify a fresh clone + `docker compose up` works against the public image (test from a clean VM)
- [ ] Flip GitHub repo visibility to Public
- [ ] Announce (optional): personal channels, /r/selfhosted, lobste.rs, Mastodon

---

## Sprint 4 — Polish (Post-Public, Continuous)

Lower-priority cleanups. Pick off as energy allows.

### 4.1 — Split `badges.rs` into a module [Polish]

`src/badges.rs` is 1344 lines, ~30 badge implementations.

- [ ] Create `src/badges/` directory:
  ```
  src/badges/
    mod.rs        — Badge trait, BadgeDisplay, BadgeContext, registry()
    daily_max.rs
    discipline.rs
    streak.rs
    rank_delta.rs
    champion.rs
    underdog.rs
    …one file per badge…
  ```
- [ ] Move each badge struct + its tests into its own file
- [ ] `registry()` stays in `mod.rs`, lists badges in display order
- [ ] Confirm `cargo test --lib badges` green

### 4.2 — Group `AppState` fields [Polish]

13 flat fields → grouped structs:

- [ ] `AppState { repos, static_assets, config, runtime, dev_mode }`
- [ ] `StaticAssets { jerseys, translations }`
- [ ] `AppConfig { base_url, signal_api_url, signal_from_number, signal_group_id, smtp_config }` — note: per CLAUDE.md, Signal config is moving to per-league `LeagueConfig`, so the AppState-level Signal fields may shrink to just `SIGNAL_API_URL` (kept global)
- [ ] `AppRuntime { http_client, concurrency_limit, mock_now }`

**Acceptance:** Test fixtures get smaller; handler signatures unchanged.

### 4.3 — Centralise route constants [Polish]

- [ ] Create `src/routes.rs` with `const`/`fn` route paths
- [ ] Use everywhere in `main.rs` and template `hx-*` attributes (via Askama context)

### 4.4 — Per-language notification rendering [Polish]

`SignalNotifier` and `mail` currently render in one fixed language.

- [ ] Notifier takes a `NotificationKind` enum + per-user/per-league language
- [ ] Rendering picks the right FTL bundle
- [ ] Test: same notification kind in `de`/`en` produces different strings

### 4.5 — Memory repos behind feature flag (if not done in 1.3) [Polish]

See Sprint 1.3 — finalise the `memory-repos` feature gating if it was deferred.

### 4.6 — Doc-site (optional, post-v0.1) [Polish]

- [ ] `mdbook` or `mkdocs` site published via GitHub Pages
- [ ] Pulls from `doc/` and rendered as a navigable docs site

---

## Decision Log

Resolved:

- [x] **Sprint 2.4** — Squash migrations into a single init file. No prod DB exists; clean starting point preferred.
- [x] **Sprint 3.2** — Commit message style: **Conventional Commits** (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`, …). Caveman style is rejected for human-readable history.
- [x] **Sprint 3.4** — Delete `handoff/` entirely. Design artefacts are not preserved.
- [x] **Sprint 4.2** — Do the `AppState` restructure now (Sprint 1), before contributors land and the churn cost goes up.

---

## Quick Reference

Mandatory gates before any PR merges (set up in CI in Sprint 3.6):

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo sqlx prepare      # if any sqlx::query! changed; commit .sqlx/
cargo audit             # no unpatched advisories
cargo deny check        # license + source allowlist
```
