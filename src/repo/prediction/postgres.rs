// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Postgres implementation of [`PredictionRepo`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    FinishedPredictionJoin, LeaderboardPredictionRow, OtherUserPrediction, PredictionRepo,
};
use crate::repo::{RepoError, RepoResult};
use crate::stage::Stage;

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

    async fn count_user_started(&self, user_id: Uuid, now: DateTime<Utc>) -> RepoResult<i64> {
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

    async fn list_finished_join(&self, league_id: Uuid) -> RepoResult<Vec<FinishedPredictionJoin>> {
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
