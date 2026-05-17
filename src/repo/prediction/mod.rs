// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Per-match prediction persistence.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::RepoResult;
use crate::stage::Stage;

mod memory;
mod postgres;

pub use memory::{FakeFinishedRow, FakeLeaderboardRow, MemoryPredictionRepo};
pub use postgres::PgPredictionRepo;

/// One other user's tip for a locked match — used to render the "andere
/// Tipps" panel without exposing them while the match is still open.
#[derive(Debug, Clone)]
pub struct OtherUserPrediction {
    pub match_id: i32,
    pub user_name: String,
    pub predicted_home: i32,
    pub predicted_away: i32,
}

/// Joined row used by the badge engine: every finished match × the user
/// who tipped it.
#[derive(Debug, Clone)]
pub struct FinishedPredictionJoin {
    pub user_id: Uuid,
    pub match_id: i32,
    pub stage: Stage,
    pub kickoff: DateTime<Utc>,
    pub score_home: i32,
    pub score_away: i32,
    pub predicted_home: i32,
    pub predicted_away: i32,
}

/// Tuple needed by the leaderboard service: who tipped what, plus the
/// match's lifecycle data.
#[derive(Debug, Clone)]
pub struct LeaderboardPredictionRow {
    pub user_name: String,
    pub stage: Stage,
    pub kickoff_time: Option<DateTime<Utc>>,
    pub status: String,
    pub score_home: Option<i32>,
    pub score_away: Option<i32>,
    pub predicted_home: i32,
    pub predicted_away: i32,
}

#[async_trait]
pub trait PredictionRepo: Send + Sync {
    /// Insert or overwrite a user's prediction for a match. The caller is
    /// responsible for checking whether the match is still open before
    /// calling this method.
    async fn upsert(
        &self,
        user_id: Uuid,
        match_id: i32,
        predicted_home: i32,
        predicted_away: i32,
    ) -> RepoResult<()>;

    /// Tips by other users (in the same league) on matches that are already
    /// locked. Never reveals tips before lock and never crosses league
    /// boundaries.
    async fn list_other_users_locked(
        &self,
        viewer_user_id: Uuid,
        league_id: Uuid,
        now: DateTime<Utc>,
    ) -> RepoResult<Vec<OtherUserPrediction>>;

    /// Numerator of the "Tippmoral" badge: how many of the started matches
    /// did this user actually tip on?
    async fn count_user_started(&self, user_id: Uuid, now: DateTime<Utc>) -> RepoResult<i64>;

    /// Every finished match × every user prediction in the given league —
    /// input to badges and other aggregate stats. Filtered by league so
    /// per-user computations (rank, matchday wins) compare only against
    /// league-mates.
    async fn list_finished_join(&self, league_id: Uuid) -> RepoResult<Vec<FinishedPredictionJoin>>;

    /// Predictions of users in `league_id` joined with user name and match —
    /// feeds the leaderboard calculation.
    async fn list_leaderboard_join(
        &self,
        league_id: Uuid,
    ) -> RepoResult<Vec<LeaderboardPredictionRow>>;
}
