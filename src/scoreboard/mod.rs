//! Provider-agnostic sports-data abstraction.
//!
//! The worker depends only on the [`ScoreboardClient`] trait and its
//! associated DTOs — never on a concrete provider's response shape. The
//! ESPN scoreboard implementation lives in [`espn`]; swapping to a
//! commercial source later means writing a second `impl ScoreboardClient`
//! and rewiring `main.rs`, with no worker code change.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use std::sync::Mutex;

use crate::stage::Stage;

pub mod espn;

pub use espn::EspnClient;

/// Lifecycle of a fixture as seen by the upstream provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchStatus {
    Scheduled,
    Live,
    Finished,
}

impl MatchStatus {
    /// String representation persisted in the `matches.status` column. Kept
    /// as a method (rather than a `Display` impl) so callers can reach for
    /// the canonical DB value without the risk of formatting drift.
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Live => "live",
            Self::Finished => "finished",
        }
    }
}

/// One team as the provider sees it. `provider_team_id` is the upstream
/// numeric id used as the primary key in the `teams` table.
#[derive(Debug, Clone)]
pub struct SportsTeam {
    pub provider_team_id: i32,
    pub display_name: String,
    pub short_name: Option<String>,
    /// ISO-3166 alpha-2 (or sub-country) flag code. The provider is
    /// responsible for any abbreviation→ISO mapping.
    pub flag_code: Option<String>,
}

/// One fixture as the provider sees it. Already classified into our
/// internal `Stage` enum and pre-mapped to clean DB-shaped values; the
/// worker just upserts.
#[derive(Debug, Clone)]
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

pub type ProviderError = Box<dyn std::error::Error + Send + Sync>;

#[async_trait]
pub trait ScoreboardClient: Send + Sync {
    /// All fixtures the provider knows about for the given UTC calendar
    /// date. An empty list is a valid response (most off-days during the
    /// tournament window).
    async fn fetch_events(&self, date: NaiveDate) -> Result<Vec<SportsEvent>, ProviderError>;
}

// ─── In-memory fake ──────────────────────────────────────────────────────────

/// Test double for `ScoreboardClient`. Stores a per-date event list and
/// an optional error for the next call (so tests can assert that the
/// worker treats fetch failures as warnings rather than crashes).
#[derive(Default)]
pub struct FakeScoreboardClient {
    inner: Mutex<FakeState>,
}

#[derive(Default)]
struct FakeState {
    events_by_date: std::collections::HashMap<NaiveDate, Vec<SportsEvent>>,
    /// Dates that should fail. Removed on first read so callers can decide
    /// whether to keep failing or recover on retry.
    fail_once: std::collections::HashSet<NaiveDate>,
    /// Total number of `fetch_events` invocations — useful to assert that
    /// the worker iterated the full window.
    pub call_count: usize,
}

impl FakeScoreboardClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed(&self, date: NaiveDate, events: Vec<SportsEvent>) {
        self.inner
            .lock()
            .unwrap()
            .events_by_date
            .insert(date, events);
    }

    /// Make the next `fetch_events(date)` call fail; subsequent calls
    /// succeed normally.
    pub fn fail_once(&self, date: NaiveDate) {
        self.inner.lock().unwrap().fail_once.insert(date);
    }

    pub fn call_count(&self) -> usize {
        self.inner.lock().unwrap().call_count
    }
}

#[async_trait]
impl ScoreboardClient for FakeScoreboardClient {
    async fn fetch_events(&self, date: NaiveDate) -> Result<Vec<SportsEvent>, ProviderError> {
        let mut s = self.inner.lock().unwrap();
        s.call_count += 1;
        if s.fail_once.remove(&date) {
            return Err(format!("simulated fetch failure for {date}").into());
        }
        Ok(s.events_by_date.get(&date).cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod fake_tests {
    use super::*;
    use chrono::TimeZone;

    fn ev(id: i64) -> SportsEvent {
        SportsEvent {
            provider_event_id: id,
            stage: Stage::Group,
            group_letter: Some("A".into()),
            home_team: None,
            away_team: None,
            score_home: None,
            score_away: None,
            kickoff: Some(Utc.with_ymd_and_hms(2026, 6, 11, 18, 0, 0).unwrap()),
            status: MatchStatus::Scheduled,
        }
    }

    #[tokio::test]
    async fn returns_seeded_events_for_date() {
        let c = FakeScoreboardClient::new();
        let d = NaiveDate::from_ymd_opt(2026, 6, 11).unwrap();
        c.seed(d, vec![ev(1), ev(2)]);
        let got = c.fetch_events(d).await.unwrap();
        assert_eq!(got.len(), 2);
    }

    #[tokio::test]
    async fn returns_empty_for_unseeded_date() {
        let c = FakeScoreboardClient::new();
        let d = NaiveDate::from_ymd_opt(2026, 6, 12).unwrap();
        assert!(c.fetch_events(d).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fail_once_is_consumed_on_first_call_only() {
        let c = FakeScoreboardClient::new();
        let d = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();
        c.fail_once(d);
        assert!(c.fetch_events(d).await.is_err());
        assert!(c.fetch_events(d).await.is_ok());
    }
}
