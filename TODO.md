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

### 1.1 — Move hardcoded German strings into FTL locales [Blocker]

Largest single task in this sprint. CLAUDE.md mandates that every user-facing
string lives in `locales/{de,en,es,fr}.ftl`. The audit found 19 violations
across 7 files.

**Files & strings to migrate:**

- [ ] `src/handlers/auth.rs:33` — error message "Ungültiger oder abgelaufener Link."
- [ ] `src/handlers/admin.rs` — "Du kannst dich nicht selbst löschen." and surrounding errors
- [ ] `src/handlers/predictions.rs` — ×4 strings: "Begegnung steht noch nicht fest…", "Ungültiger Tipp: …" and variants
- [ ] `src/handlers/dev.rs:529` — "Kein zukünftiges Spiel gefunden…" (dev-only — acceptable to keep English-only since dev mode is admin/dev-facing, but be consistent)
- [ ] `src/notifier.rs` — ×2: Weltmeister-Tipp reminder + stage-label notification templates. Refactor to take a `NotificationKind` enum, render via FTL in the notifier impl.
- [ ] `src/mail.rs` — invite/reminder mail bodies ("Bookmarke diesen Link…", "(Anpfiff Eröffnungsspiel)…"). Move subjects and bodies into FTL with parameter substitution.
- [ ] `src/badges.rs` — ×8 German doc comments and badge descriptions. Doc comments → English. User-visible badge titles/descriptions → FTL.
- [ ] `src/jersey.rs` — jersey names (e.g. "Dänemark", "Dänemark Away"). Two options: (a) keep canonical English names in the preset map, render translated label via FTL key `jersey-name-{preset_id}`; (b) accept that jersey display names are i18n keys. Option (a) is cleaner.

**For each migrated string:**
1. Add a key + `# translator comment` to all four FTL files (`de.ftl` `en.ftl` `es.ftl` `fr.ftl`).
2. Replace the hardcoded string with `t.get("key-name")` (or the parameterised variant).
3. If parameters: use Fluent placeables `{ $name }`.

**Acceptance:** `grep -rE '"[A-ZÄÖÜ][a-zäöüß]+ [a-zäöüß]' src/` returns no user-facing German strings; doc comments are English; `cargo test` green.

**Test gate:** Add at least one integration test per migrated handler that asserts the response contains the expected English string when `users.language = 'en'`.

### 1.2 — Eliminate `.unwrap()` in response paths [Blocker]

- [ ] `src/handlers/jersey.rs:175` — `"/".parse().unwrap()` → `.expect("static HX-Location header value is valid")` (or use `HeaderValue::from_static("/")` which is infallible).
- [ ] `src/handlers/services.rs:192` — `jerseys.get("classic").unwrap()` → either fall back to first preset, or `.expect("classic preset must exist in jersey config")` and document the invariant in `jersey.rs`. Add a startup assertion in `jersey::load()` that "classic" key exists.
- [ ] Sweep: `grep -nE '\.unwrap\(\)' src/handlers/ src/repo/ src/worker.rs src/notifier.rs` — for every remaining hit, decide: justifiable invariant → `.expect("why")`; otherwise → propagate error with `?` and proper `Result` type.

**Acceptance:** No bare `.unwrap()` in `src/handlers/`, `src/notifier.rs`, `src/worker.rs` (startup in `main.rs` and `jersey::load()` are exempt — document why).

### 1.3 — Split repo modules: trait / postgres / memory [Polish, but worth it]

Four files mix trait definition, Postgres impl, in-memory fake, and tests:

- [ ] `src/repo/user.rs` (772 lines) → `src/repo/user/{mod.rs, postgres.rs, memory.rs}`
- [ ] `src/repo/match_.rs` (743 lines) → `src/repo/match_/{mod.rs, postgres.rs, memory.rs}`
- [ ] `src/repo/notification.rs` (650 lines) → `src/repo/notification/{mod.rs, postgres.rs, memory.rs}`
- [ ] `src/repo/prediction.rs` (483 lines) → `src/repo/prediction/{mod.rs, postgres.rs, memory.rs}`

**Pattern for each module:**
```
src/repo/<name>/
  mod.rs        — trait, error types, re-exports
  postgres.rs   — Pg<Name>Repo impl
  memory.rs     — Memory<Name>Repo impl, gated by #[cfg(any(test, feature = "memory-repos"))]
```

- [ ] Decide whether to gate in-memory fakes behind a `memory-repos` Cargo feature so they do not ship in the production binary. If yes, add `[features] memory-repos = []` to `Cargo.toml` and enable it in `[dev-dependencies]` and CI test runs. Otherwise leave as `#[cfg(test)]` only and remove their `pub` visibility from non-test code.

**Acceptance:** `cargo build --release` produces a binary that does not contain `Memory*Repo` symbols (verify with `nm target/release/pila | grep -i memory`); all tests still pass.

### 1.4 — Fix `AppState.db: Option<PgPool>` smell [Polish]

The `Option<PgPool>` exists only because `setup_post` runs a multi-table
transaction outside the repo layer.

- [ ] Introduce `repo::bootstrap::BootstrapRepo` trait with one method:
  `create_first_league_and_admin(name, league_name, …) -> Result<UserId>`
- [ ] Implement for `PgBootstrapRepo` (encapsulates the existing multi-table tx)
- [ ] Wire into `Repos` struct
- [ ] Replace direct `state.db` calls in `handlers/setup.rs` with the new repo method
- [ ] Remove `AppState.db` field

**Acceptance:** `grep -n 'state\.db' src/` returns no hits except possibly `main.rs` setup; `cargo test` green; `/setup` integration test (added in 1.5) passes.

### 1.5 — Close test coverage gaps [Blocker]

Add integration tests under `tests/` for paths currently uncovered:

- [ ] `tests/handler_setup.rs` — Happy path: POST `/setup` with valid form creates league + first user + cookie. Edge cases: setup already done (returns 403/redirect), missing required fields, invalid email format.
- [ ] `tests/handler_language.rs` — POST `/profile/language` with each of `de`, `en`, `es`, `fr` persists and returns `HX-Location: /`. Invalid locale (e.g. `xx`) is rejected.
- [ ] `tests/handler_auth.rs` — Magic link flow: valid token sets cookie + redirects; unknown token → 401/404; tampered cookie → 401; missing cookie on protected route → redirect to error page.
- [ ] `tests/handler_leaderboard.rs` — Returns 200 with the right users, in the right order, scoped to the caller's league. Cross-league isolation (extend `multi_league_isolation.rs` if more natural there).
- [ ] `tests/handler_services.rs` — Leaderboard JSON endpoint (if exposed). Verify shape + league scope.

**Acceptance:** `cargo test --all-targets` passes; `cargo llvm-cov --html` (optional) shows the new files covered.

### 1.6 — Refactor `dev_simulate_next_matchday` [Polish]

Currently 127 lines in `src/handlers/dev.rs:344–470`. Mixes match selection,
RNG, time mutation, and bulk updates.

- [ ] Extract `fn find_next_unstarted_matchday(matches: &[Match]) -> Option<Vec<Match>>` (pure, testable)
- [ ] Extract `fn random_result(rng: &mut impl Rng) -> (i32, i32)` (pure, testable)
- [ ] Unit-test both extracted helpers
- [ ] Handler shrinks to orchestration only

**Acceptance:** Handler ≤ 50 lines; two new unit tests in `dev.rs`; behaviour unchanged (verify by running `/dev/simulate/next-matchday` manually).

### 1.7 — Add missing doc comments on public trait methods [Polish]

- [ ] `src/repo/league.rs` — `LeagueRepo::get_config()`, `set_config_*()`, `list_*()`
- [ ] `src/repo/user.rs` — `set_jersey()`, `set_email()`, `set_language()` and any other undocumented trait methods
- [ ] `src/repo/notification.rs` — verify all trait methods documented (idempotency contract is critical)
- [ ] `src/repo/prediction.rs`, `src/repo/match_.rs`, `src/repo/special_prediction.rs`, `src/repo/team.rs`, `src/repo/settings.rs` — same sweep

**Acceptance:** `cargo doc --no-deps --document-private-items 2>&1 | grep -i 'missing'` is empty for all `pub fn` on traits.

### 1.8 — Sprint 1 wrap-up [Blocker]

- [ ] Run `cargo clippy --all-targets -- -D warnings` — clean
- [ ] Run `cargo fmt` — clean diff
- [ ] Run `cargo sqlx prepare` and commit `.sqlx/` if any query changed
- [ ] Update CHANGELOG.md (create if missing) with v0.1.0-unreleased entries
- [ ] Tag a working snapshot: `git tag v0.1.0-rc1` (do not push yet)

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

- [ ] `cargo fmt --check` step
- [ ] `cargo audit` (RustSec advisory DB) — fail on any unpatched advisory
- [ ] `cargo deny check` (license + source allowlist + dupes). Add a `deny.toml` that:
  - Allowlists AGPL-compatible licenses for deps
  - Bans known-bad crates
  - Warns on duplicate versions
- [ ] Enable Dependabot (`.github/dependabot.yml`) for `cargo` and `github-actions` ecosystems, weekly schedule
- [ ] (Optional) Coverage step with `cargo-llvm-cov` and upload to Codecov

### 3.7 — README polish for public eyeballs [Public-blocker]

- [ ] Add a screenshot or short GIF of the UI at the top
- [ ] Add a "Why Pila?" paragraph (vs. existing prediction games — fewer ads, self-hostable, multi-league, AGPL)
- [ ] Add a "Roadmap" section: what's coming (post-v0.1 ideas) and what's intentionally out of scope
- [ ] Add badges row: CI (exists), license (AGPL), Rust edition, GHCR image tag (once built)
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
