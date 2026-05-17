// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Match persistence.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::RepoResult;
use crate::stage::Stage;

mod memory;
mod postgres;

pub use memory::{FakeMatch, MemoryMatchRepo};
pub use postgres::PgMatchRepo;

/// Row used to render the index page — every match the user might see, with
/// the user's own prediction merged in via LEFT JOIN.
#[derive(Debug, Clone)]
pub struct IndexMatchRow {
    pub id: i32,
    pub stage: Stage,
    pub group_letter: Option<String>,
    pub kickoff_time: Option<DateTime<Utc>>,
    pub status: String,
    pub score_home: Option<i32>,
    pub score_away: Option<i32>,
    pub team_home_id: Option<i32>,
    pub team_away_id: Option<i32>,
    pub home_name: String,
    pub away_name: String,
    pub home_flag: Option<String>,
    pub away_flag: Option<String>,
    pub predicted_home: Option<i32>,
    pub predicted_away: Option<i32>,
}

/// Information needed to validate a tip POST: when does the match start, and
/// are both teams known? Returned as `None` if the match id is unknown.
#[derive(Debug, Clone)]
pub struct MatchLockInfo {
    pub kickoff_time: Option<DateTime<Utc>>,
    pub team_home_id: Option<i32>,
    pub team_away_id: Option<i32>,
    pub stage: Stage,
    pub home_name: String,
    pub away_name: String,
    pub home_flag_code: Option<String>,
    pub away_flag_code: Option<String>,
}

/// One scored row of a finished group-stage match, joined with team display
/// data — the input to the standings calculator.
#[derive(Debug, Clone)]
pub struct FinishedGroupMatch {
    pub group_letter: String,
    pub home_id: i32,
    pub away_id: i32,
    pub score_home: i32,
    pub score_away: i32,
    pub home_name: String,
    pub home_flag: Option<String>,
    pub away_name: String,
    pub away_flag: Option<String>,
}

/// Payload for upserting one match coming back from the ESPN scoreboard.
/// All fields except `espn_event_id` and `stage` may flip back to `None`
/// across worker ticks (e.g. when ESPN walks back a TBD bracket slot), so
/// the SQL upsert uses `COALESCE` to preserve previously-seen values.
#[derive(Debug, Clone)]
pub struct EspnMatchUpsert<'a> {
    pub espn_event_id: i64,
    pub stage: Stage,
    pub group_letter: Option<&'a str>,
    pub team_home_id: Option<i32>,
    pub team_away_id: Option<i32>,
    pub score_home: Option<i32>,
    pub score_away: Option<i32>,
    pub kickoff_time: Option<DateTime<Utc>>,
    pub status: &'a str,
}

#[async_trait]
pub trait MatchRepo: Send + Sync {
    /// All matches with the given user's predictions merged in via LEFT JOIN.
    /// Returns every match regardless of stage or status.
    async fn list_for_index(&self, user_id: Uuid) -> RepoResult<Vec<IndexMatchRow>>;
    /// Lock-check data for the given match. Returns `None` if the match id
    /// is unknown.
    async fn find_lock_info(&self, match_id: i32) -> RepoResult<Option<MatchLockInfo>>;
    /// Earliest kickoff across all matches with both teams set. `None` if no
    /// such match exists. Used as the champion-pick lock threshold.
    async fn first_kickoff(&self) -> RepoResult<Option<DateTime<Utc>>>;
    /// Earliest kickoff for knockout-stage matches. `None` if no knockout
    /// matches exist yet.
    async fn first_knockout_kickoff(&self) -> RepoResult<Option<DateTime<Utc>>>;
    /// Team id of the winner of the final, or `None` if the final has not
    /// finished.
    async fn actual_champion(&self) -> RepoResult<Option<i32>>;
    /// All finished group-stage matches with team display data — input to
    /// the group standings calculator.
    async fn finished_group_rows(&self) -> RepoResult<Vec<FinishedGroupMatch>>;
    /// Count of matches with both teams known whose kickoff has already
    /// passed — used as the denominator of the "Tippmoral" badge.
    async fn started_with_both_teams_count(&self, now: DateTime<Utc>) -> RepoResult<i64>;

    /// Upsert one match from the ESPN sync. Idempotent on `espn_event_id`.
    async fn upsert_from_espn(&self, upsert: EspnMatchUpsert<'_>) -> RepoResult<()>;

    /// Update match result and status (dev mode only).
    /// Used for simulating tournament progression.
    async fn update_result(
        &self,
        match_id: i32,
        score_home: Option<i32>,
        score_away: Option<i32>,
        status: &str,
    ) -> RepoResult<()>;
}
