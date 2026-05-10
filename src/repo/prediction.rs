//! Per-match prediction persistence.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::sync::Mutex;
use uuid::Uuid;

use super::{RepoError, RepoResult};
use crate::stage::Stage;

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
    async fn count_user_started(
        &self,
        user_id: Uuid,
        now: DateTime<Utc>,
    ) -> RepoResult<i64>;

    /// Every finished match × every user prediction in the given league —
    /// input to badges and other aggregate stats. Filtered by league so
    /// per-user computations (rank, matchday wins) compare only against
    /// league-mates.
    async fn list_finished_join(
        &self,
        league_id: Uuid,
    ) -> RepoResult<Vec<FinishedPredictionJoin>>;

    /// Predictions of users in `league_id` joined with user name and match —
    /// feeds the leaderboard calculation.
    async fn list_leaderboard_join(
        &self,
        league_id: Uuid,
    ) -> RepoResult<Vec<LeaderboardPredictionRow>>;
}

// ─── Postgres implementation ─────────────────────────────────────────────────

pub struct PgPredictionRepo {
    pool: PgPool,
}

impl PgPredictionRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PredictionRepo for PgPredictionRepo {
    async fn upsert(
        &self,
        user_id: Uuid,
        match_id: i32,
        predicted_home: i32,
        predicted_away: i32,
    ) -> RepoResult<()> {
        sqlx::query!(
            r#"
            INSERT INTO predictions (user_id, match_id, predicted_home, predicted_away, updated_at)
            VALUES ($1, $2, $3, $4, NOW())
            ON CONFLICT (user_id, match_id) DO UPDATE SET
                predicted_home = EXCLUDED.predicted_home,
                predicted_away = EXCLUDED.predicted_away,
                updated_at = NOW()
            "#,
            user_id,
            match_id,
            predicted_home,
            predicted_away
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }

    async fn list_other_users_locked(
        &self,
        viewer_user_id: Uuid,
        league_id: Uuid,
        now: DateTime<Utc>,
    ) -> RepoResult<Vec<OtherUserPrediction>> {
        let rows = sqlx::query!(
            r#"
            SELECT p.match_id, u.name as user_name,
                   p.predicted_home, p.predicted_away
            FROM predictions p
            JOIN users u ON u.id = p.user_id
            JOIN matches m ON m.id = p.match_id
            WHERE m.kickoff_time IS NOT NULL
              AND m.kickoff_time < $1
              AND u.id != $2
              AND u.league_id = $3
            ORDER BY u.name
            "#,
            now,
            viewer_user_id,
            league_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(rows
            .into_iter()
            .map(|r| OtherUserPrediction {
                match_id: r.match_id,
                user_name: r.user_name,
                predicted_home: r.predicted_home,
                predicted_away: r.predicted_away,
            })
            .collect())
    }

    async fn count_user_started(
        &self,
        user_id: Uuid,
        now: DateTime<Utc>,
    ) -> RepoResult<i64> {
        let c = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "c!"
            FROM predictions p
            JOIN matches m ON m.id = p.match_id
            WHERE p.user_id = $1
              AND m.kickoff_time IS NOT NULL
              AND m.kickoff_time < $2
              AND m.team_home_id IS NOT NULL
              AND m.team_away_id IS NOT NULL
            "#,
            user_id,
            now
        )
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(c)
    }

    async fn list_finished_join(
        &self,
        league_id: Uuid,
    ) -> RepoResult<Vec<FinishedPredictionJoin>> {
        let rows = sqlx::query!(
            r#"
            SELECT p.user_id,
                   m.id as match_id,
                   m.stage as "stage: Stage",
                   m.kickoff_time as "kickoff!",
                   m.score_home as "score_home!",
                   m.score_away as "score_away!",
                   p.predicted_home,
                   p.predicted_away
            FROM predictions p
            JOIN users u ON u.id = p.user_id
            JOIN matches m ON m.id = p.match_id
            WHERE m.status = 'finished'
              AND m.score_home IS NOT NULL
              AND m.score_away IS NOT NULL
              AND m.kickoff_time IS NOT NULL
              AND u.league_id = $1
            "#,
            league_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(rows
            .into_iter()
            .map(|r| FinishedPredictionJoin {
                user_id: r.user_id,
                match_id: r.match_id,
                stage: r.stage,
                kickoff: r.kickoff,
                score_home: r.score_home,
                score_away: r.score_away,
                predicted_home: r.predicted_home,
                predicted_away: r.predicted_away,
            })
            .collect())
    }

    async fn list_leaderboard_join(
        &self,
        league_id: Uuid,
    ) -> RepoResult<Vec<LeaderboardPredictionRow>> {
        let rows = sqlx::query!(
            r#"
            SELECT u.name,
                   m.stage as "stage: Stage",
                   m.kickoff_time,
                   m.status,
                   m.score_home as "score_home?",
                   m.score_away as "score_away?",
                   p.predicted_home,
                   p.predicted_away
            FROM predictions p
            JOIN users u ON u.id = p.user_id
            JOIN matches m ON m.id = p.match_id
            WHERE u.league_id = $1
            "#,
            league_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(rows
            .into_iter()
            .map(|r| LeaderboardPredictionRow {
                user_name: r.name,
                stage: r.stage,
                kickoff_time: r.kickoff_time,
                status: r.status,
                score_home: r.score_home,
                score_away: r.score_away,
                predicted_home: r.predicted_home,
                predicted_away: r.predicted_away,
            })
            .collect())
    }
}

// ─── In-memory fake ──────────────────────────────────────────────────────────

/// Test-only seed for `list_finished_join` — multi-league isolation tests
/// drive `BadgeContext` from these rows.
#[derive(Debug, Clone)]
pub struct FakeFinishedRow {
    pub user_id: Uuid,
    pub league_id: Uuid,
    pub match_id: i32,
    pub stage: Stage,
    pub kickoff: DateTime<Utc>,
    pub score_home: i32,
    pub score_away: i32,
    pub predicted_home: i32,
    pub predicted_away: i32,
}

/// Test-only seed for `list_leaderboard_join` — drives the leaderboard
/// service in fake-backed integration tests.
#[derive(Debug, Clone)]
pub struct FakeLeaderboardRow {
    pub league_id: Uuid,
    pub user_name: String,
    pub stage: Stage,
    pub kickoff_time: Option<DateTime<Utc>>,
    pub status: String,
    pub score_home: Option<i32>,
    pub score_away: Option<i32>,
    pub predicted_home: i32,
    pub predicted_away: i32,
}

#[derive(Default)]
struct MemoryPredictionState {
    rows: Vec<(Uuid, i32, i32, i32)>,
    finished: Vec<FakeFinishedRow>,
    leaderboard: Vec<FakeLeaderboardRow>,
}

#[derive(Default)]
pub struct MemoryPredictionRepo {
    inner: Mutex<MemoryPredictionState>,
}

impl MemoryPredictionRepo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read every stored prediction — useful for handler tests asserting
    /// that an upsert actually landed.
    pub fn all(&self) -> Vec<(Uuid, i32, i32, i32)> {
        self.inner.lock().unwrap().rows.clone()
    }

    /// Seed a row that surfaces via `list_finished_join` for the given league.
    pub fn seed_finished(&self, row: FakeFinishedRow) {
        self.inner.lock().unwrap().finished.push(row);
    }

    /// Seed a row that surfaces via `list_leaderboard_join` for the given league.
    pub fn seed_leaderboard(&self, row: FakeLeaderboardRow) {
        self.inner.lock().unwrap().leaderboard.push(row);
    }
}

#[async_trait]
impl PredictionRepo for MemoryPredictionRepo {
    async fn upsert(
        &self,
        user_id: Uuid,
        match_id: i32,
        predicted_home: i32,
        predicted_away: i32,
    ) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        if let Some(r) = s
            .rows
            .iter_mut()
            .find(|r| r.0 == user_id && r.1 == match_id)
        {
            r.2 = predicted_home;
            r.3 = predicted_away;
        } else {
            s.rows.push((user_id, match_id, predicted_home, predicted_away));
        }
        Ok(())
    }

    async fn list_other_users_locked(
        &self,
        _viewer_user_id: Uuid,
        _league_id: Uuid,
        _now: DateTime<Utc>,
    ) -> RepoResult<Vec<OtherUserPrediction>> {
        // The fake intentionally returns empty here — matches needed to gate
        // visibility live in `MemoryMatchRepo` and tests that need this path
        // should exercise the Postgres impl instead.
        Ok(Vec::new())
    }

    async fn count_user_started(
        &self,
        user_id: Uuid,
        _now: DateTime<Utc>,
    ) -> RepoResult<i64> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .rows
            .iter()
            .filter(|r| r.0 == user_id)
            .count() as i64)
    }

    async fn list_finished_join(
        &self,
        league_id: Uuid,
    ) -> RepoResult<Vec<FinishedPredictionJoin>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .finished
            .iter()
            .filter(|r| r.league_id == league_id)
            .map(|r| FinishedPredictionJoin {
                user_id: r.user_id,
                match_id: r.match_id,
                stage: r.stage,
                kickoff: r.kickoff,
                score_home: r.score_home,
                score_away: r.score_away,
                predicted_home: r.predicted_home,
                predicted_away: r.predicted_away,
            })
            .collect())
    }

    async fn list_leaderboard_join(
        &self,
        league_id: Uuid,
    ) -> RepoResult<Vec<LeaderboardPredictionRow>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .leaderboard
            .iter()
            .filter(|r| r.league_id == league_id)
            .map(|r| LeaderboardPredictionRow {
                user_name: r.user_name.clone(),
                stage: r.stage,
                kickoff_time: r.kickoff_time,
                status: r.status.clone(),
                score_home: r.score_home,
                score_away: r.score_away,
                predicted_home: r.predicted_home,
                predicted_away: r.predicted_away,
            })
            .collect())
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[tokio::test]
    async fn upsert_inserts_new_row() {
        let repo = MemoryPredictionRepo::new();
        let user_id = Uuid::new_v4();
        repo.upsert(user_id, 7, 2, 1).await.unwrap();
        let all = repo.all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], (user_id, 7, 2, 1));
    }

    #[tokio::test]
    async fn upsert_overwrites_existing() {
        let repo = MemoryPredictionRepo::new();
        let user_id = Uuid::new_v4();
        repo.upsert(user_id, 7, 2, 1).await.unwrap();
        repo.upsert(user_id, 7, 3, 0).await.unwrap();
        let all = repo.all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], (user_id, 7, 3, 0));
    }

    #[tokio::test]
    async fn upsert_keeps_predictions_per_user_separate() {
        let repo = MemoryPredictionRepo::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        repo.upsert(a, 1, 1, 0).await.unwrap();
        repo.upsert(b, 1, 0, 1).await.unwrap();
        assert_eq!(repo.all().len(), 2);
    }
}
