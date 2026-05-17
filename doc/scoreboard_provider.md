# Scoreboard Provider Contract

This document is the canonical specification for any implementation of
`pila::scoreboard::ScoreboardClient`. It captures the assumptions the
background worker (`src/worker.rs`) makes about the shape and timing of
the data returned, so that a second provider (e.g. a commercial sports
data API) can be plugged in without re-reading the worker code.

> **Maintenance rule:** every change to the trait, its DTOs, or the
> contracts below must be reflected here in the same commit.
> [`architecture.md`](./architecture.md) points here so the file stays
> the single source of truth for provider behaviour.

---

## 1. Trait

```rust
#[async_trait]
pub trait ScoreboardClient: Send + Sync {
    async fn fetch_events(&self, date: NaiveDate) -> Result<Vec<SportsEvent>, ProviderError>;
}
```

`ProviderError` is `Box<dyn std::error::Error + Send + Sync>` —
intentionally broad. The worker never inspects the variant, only logs it.

### Invocation pattern

The worker iterates `WC_WINDOW_START..=WC_WINDOW_END` (default
`2026-06-01` → `2026-07-25`, both inclusive) once per 30-minute tick and
calls `fetch_events(date)` for every UTC calendar date in the window.
That is **dozens of calls per tick** — providers should cache where
sensible (see §6).

`Send + Sync` is mandatory: the client is held inside an
`Arc<dyn ScoreboardClient>` and shared across the worker's tokio task.

---

## 2. DTOs

### `SportsEvent`

```rust
pub struct SportsEvent {
    pub provider_event_id: i64,
    pub stage: Stage,
    pub group_letter: Option<String>,
    pub home_team: Option<SportsTeam>,
    pub away_team: Option<SportsTeam>,
    pub score_home: Option<i32>,
    pub score_away: Option<i32>,
    pub kickoff: Option<DateTime<Utc>>,
    pub status: MatchStatus,
}
```

| Field                | Required for | Notes |
|----------------------|--------------|-------|
| `provider_event_id`  | always       | Stable identifier of the fixture across all calls. Persisted as `matches.espn_event_id` (the column name is historical). Must not change for the same logical match. |
| `stage`              | always       | One of the seven `Stage` variants — see §3. |
| `group_letter`       | Stage::Group | `Some("A")`..`Some("L")`. `None` is allowed during early sync when the assignment is not yet known; later upserts can populate it. **Single uppercase ASCII letter** — the DB schema's CHECK constraint enforces this. |
| `home_team`/`away_team` | when known | Both `None` is acceptable for placeholder fixtures (e.g. `"Quarterfinal 1"` slots before the bracket is set). The worker preserves earlier non-null values via `COALESCE` semantics in the upsert. |
| `score_home`/`score_away` | finished or live | `None` until the provider reports a score. |
| `kickoff`            | always once known | UTC. `None` only if the provider truly does not yet know — otherwise must be present even for far-future matches. |
| `status`             | always       | See §4. |

### `SportsTeam`

```rust
pub struct SportsTeam {
    pub provider_team_id: i32,
    pub display_name: String,
    pub short_name: Option<String>,
    pub flag_code: Option<String>,
}
```

- `provider_team_id` is the **primary key** of `teams` — must be stable
  across all calls.
- `display_name` is rendered in the UI and Signal messages (German
  spellings are fine).
- `short_name` is the abbreviation shown in compact tables (e.g. `"GER"`).
- `flag_code` is an ISO-3166 alpha-2 code or a flagcdn.com sub-region
  code (`"gb-eng"`, `"gb-sct"`, `"gb-wls"`). The worker will build
  `https://flagcdn.com/w40/<code>.png` from it. Provider must do any
  abbreviation→ISO mapping internally — the trait API only takes ISO.

### `MatchStatus`

```rust
pub enum MatchStatus { Scheduled, Live, Finished }
```

Maps to the `matches.status` column via `MatchStatus::as_db_str()`
(`"scheduled"` / `"live"` / `"finished"`). No other values are accepted.

---

## 3. `Stage` mapping

The seven values match the WC 2026 bracket exactly:

| `Stage`           | Multiplier | When it applies |
|-------------------|-----------|-----------------|
| `Group`           | 1         | All 72 group-stage fixtures. |
| `RoundOf32`       | 2         | First knockout round (32 → 16). |
| `RoundOf16`       | 3         | 16 → 8. |
| `QuarterFinal`    | 4         | 8 → 4. |
| `SemiFinal`       | 5         | 4 → 2. |
| `ThirdPlace`      | 4         | The bronze-medal match. |
| `Final`           | 6         | The single final fixture. |

**Knockout score rule (DB-side, but worth surfacing here):** scores must
reflect the result *before* a penalty shoot-out — i.e. the result after
90 minutes plus extra time. A draw at that point is a valid stored value
and counts for "exact result" points in the scoring engine. Providers
that report a separate `penalty_shootout_score` field must drop it.

---

## 4. `MatchStatus` semantics

| Variant      | Meaning |
|--------------|---------|
| `Scheduled`  | Match has not started yet. Tips remain open until `kickoff < now()`. |
| `Live`       | Match is currently in play. Tips are locked. Score may still mutate. |
| `Finished`   | Match has reached full time (incl. extra time when applicable). Score is final. The scoring engine and `actual_champion()` only count `Finished` rows. |

Providers that report a separate "halftime" / "postponed" / "abandoned"
state must collapse them to one of the three values above. **Postponed
fixtures are `Scheduled` until rescheduled** — *never* `Finished` with a
`0:0`, since that would falsely award points.

---

## 5. Idempotency & ordering invariants

The worker upserts every event on every tick. Two consequences for
provider implementers:

1. **`provider_event_id` is the upsert key.** The same logical fixture
   must always come back with the same id. ESPN's `event.id` is stable —
   if your provider lacks a stable id, mint one deterministically (e.g.
   hash of date + home_team_id + away_team_id) and document it.
2. **Returning the same event multiple times in one tick is allowed but
   wasteful.** The worker will perform N upserts. If the provider has
   per-day endpoints (like ESPN), de-dupe inside the client.
3. **Score regression is allowed.** A live match's score can fluctuate;
   the worker writes whatever the provider reports.
4. **Team-id flips are NOT allowed.** Once `provider_team_id` X is
   reported as `"Argentina"`, it must never come back later as a
   different team. The DB schema treats team ids as immutable.

---

## 6. Caching guidance

The worker calls `fetch_events` ~55 times per 30-minute tick (once per
day in the WC window). Providers SHOULD:

- Hold a cached HTTP client (`reqwest::Client::new()` is cheap to clone
  but not free per call).
- Cache slow-moving auxiliary lookups (e.g. group-letter assignments)
  for the lifetime of the `EspnClient`-style struct. See
  `EspnClient::groups_cache` for the established pattern.
- NOT cache scoreboard responses themselves — the per-day call cost is
  low and live scores must propagate within one tick.

---

## 7. Error handling

The worker catches every `ProviderError` returned from `fetch_events`
and logs at WARN level — it does *not* abort the tick. Therefore:

- Network timeouts, malformed JSON, missing fields → return `Err`. The
  next tick will retry.
- A successful HTTP call with zero events → return `Ok(vec![])`. This is
  the normal off-day response.
- A partial response where one event is malformed but others parse
  cleanly → log + skip the bad event, return the rest. Errors are
  per-tick, per-day; do not let one rotten event take down a whole day.

Implementations MUST NOT panic on malformed upstream responses.

---

## 8. Wiring

A new provider plugs in at exactly two points:

1. **Construction** in `src/main.rs`:
   ```rust
   let scoreboard: Arc<dyn ScoreboardClient> = Arc::new(MyProvider::new(api_key));
   worker::start_background_worker(repos, scoreboard, notifier).await;
   ```
2. **Module declaration** in `src/scoreboard/mod.rs` (e.g.
   `pub mod my_provider;`) plus the `pub use` re-export if external
   callers need direct construction.

Nothing else in the codebase should change. If a refactor seems to need
worker-side changes, that's a signal the trait is wrong — update the
trait, this doc, and every implementation in the same commit.

---

## 9. Required tests for a new implementation

A new `impl ScoreboardClient` must ship with:

1. **Unit tests for the response→DTO mapping** — feed a captured raw
   response (JSON fixture) through the parser and assert the resulting
   `SportsEvent`s. See `src/scoreboard/espn.rs` `mod tests` for the
   shape.
2. **A passing test that the resulting events round-trip through
   `worker::update_data` against the in-memory `Repos`.** The existing
   `update_data_upserts_every_event_returned_by_provider` test in
   `src/worker.rs` shows the pattern via `FakeScoreboardClient`; a real
   provider can use the same fake in test code by injecting it directly
   into the worker entrypoint, or by writing an integration test that
   stands up the new client against a recorded HTTP response.
3. **One test per `Stage` value** the provider can return — easy to skip
   group/QF/SF and have a wrong heuristic ship to production. ESPN's
   classification has 13 such tests.
4. **One test per `MatchStatus` value.** Mapping the upstream
   "in-play" / "live" / "ongoing" string to `Live` is the kind of detail
   that drifts silently.
5. **A flag-code mapping test if applicable.** Providers that emit
   ISO codes directly can skip this.

Tests live alongside the impl in `src/scoreboard/<provider>.rs` under
`#[cfg(test)] mod tests`.

---

## 10. Reference implementation

`src/scoreboard/espn.rs` is the canonical implementation. Read it
end-to-end before writing a new provider — many of the rules above were
extracted from real ESPN quirks (empty `notes[]` headlines, missing
group letters needing the standings-endpoint fallback, the
`pre`/`in`/`post` state vocabulary, etc.).

Useful in-tree examples to copy:

- **Lazy auxiliary cache** — `EspnClient::groups_cache` populates on
  first call and is held in `Arc<tokio::sync::Mutex<...>>`.
- **Stage classification cascade** — `classify_slug` (authoritative)
  with `classify_stage` heuristic fallback.
- **Status mapping** — `match s.type_.state.as_str() { "in" => Live, …
  }`.

---

## 11. Out of scope

The trait is intentionally *just* the read-side scoreboard. The
following live elsewhere and are not part of the provider contract:

- Score persistence — handled by `MatchRepo::upsert_from_espn`.
- Score-to-points conversion — `src/scoring.rs`.
- Notification dispatch — `src/notifier.rs` + `NotificationRepo`.
- Magic-link auth, leaderboard rendering — handler layer only.

If you find yourself adding a method to `ScoreboardClient` that touches
any of the above, you're probably looking for a different abstraction.
